import type { TubeSettings } from "$lib/tube/types";

export type PresetId = "clean" | "focus" | "seeker";

export const PRESET_IDS: PresetId[] = ["clean", "focus", "seeker"];

const CLEAN: Partial<TubeSettings> = {
  hide_members: true,
  hide_mixes: true,
  hide_playables: true,
  hide_end_screen: true,
  hide_shorts: false,
  hide_home_feed: false,
  hide_related: false,
  hide_watched: false,
  hide_comments: false,
};

const FOCUS: Partial<TubeSettings> = {
  ...CLEAN,
  hide_home_feed: true,
  hide_related: true,
  hide_shorts: true,
  hide_watched: true,
  watched_threshold: 85,
};

const SEEKER: Partial<TubeSettings> = {
  ...FOCUS,
  hide_comments: true,
  start_page: "subscriptions",
};

const PRESETS: Record<PresetId, Partial<TubeSettings>> = { clean: CLEAN, focus: FOCUS, seeker: SEEKER };

export function presetPatch(id: PresetId): Partial<TubeSettings> {
  return { ...PRESETS[id] };
}

export function activePreset(s: TubeSettings): PresetId | null {
  for (const id of ["seeker", "focus", "clean"] as PresetId[]) {
    const p = PRESETS[id];
    let ok = true;
    for (const [k, v] of Object.entries(p)) {
      if (s[k as keyof TubeSettings] !== v) {
        ok = false;
        break;
      }
    }
    if (ok) return id;
  }
  return null;
}

export function presetDiff(s: TubeSettings, id: PresetId): number {
  let n = 0;
  for (const [k, v] of Object.entries(PRESETS[id])) if (s[k as keyof TubeSettings] !== v) n++;
  return n;
}

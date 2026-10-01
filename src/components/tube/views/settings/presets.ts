// Distraction presets of the OmniTube settings (catalog in
// estudos/102-youtube/re/_distracoes-catalogo.md, "Presets sugeridos"):
//   Clean   no members-only, mixes, playables or end screens
//   Focus   Clean + empty home, no related, no Shorts, no watched videos
//   Seeker  Focus + no comments, opens on Subscriptions
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

/** The settings patch a preset applies. A fresh copy, safe to mutate. */
export function presetPatch(id: PresetId): Partial<TubeSettings> {
  return { ...PRESETS[id] };
}

/** Which preset the current settings match exactly (most specific first), or null for "custom". */
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

/** Number of settings that would change if the preset were applied. */
export function presetDiff(s: TubeSettings, id: PresetId): number {
  let n = 0;
  for (const [k, v] of Object.entries(PRESETS[id])) if (s[k as keyof TubeSettings] !== v) n++;
  return n;
}

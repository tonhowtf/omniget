import { OMNIGET_PRESET, getAppearancePresentation, updateSettings } from "./settings-store.svelte";

/**
 * Workspace presets (palettes in workspace-design.css). This module is an
 * adapter: the settings store is the only authority for mode/preset, and the
 * legacy localStorage key is read once by the store's appearance import.
 */
export const PRESETS = [
  {
    "id": "amber-minimal",
    "name": "Amber Minimal",
    "colors": [
      "#ffffff",
      "#f59e0b",
      "#171717"
    ]
  },
  {
    "id": "orange-soft",
    "name": "Orange Soft",
    "colors": [
      "#ffffff",
      "#bd4500",
      "#121113"
    ]
  },
  {
    "id": "claude-amber",
    "name": "Claude Amber",
    "colors": [
      "#faf9f5",
      "#c96442",
      "#262624"
    ]
  },
  {
    "id": "amber-hearth",
    "name": "Amber Hearth",
    "colors": [
      "#ffffff",
      "#d87943",
      "#121113"
    ]
  },
  {
    "id": "amber-slate",
    "name": "Amber Slate",
    "colors": [
      "#e8ebed",
      "#df6035",
      "#1a1a1a"
    ]
  },
  {
    "id": "orange-vivid",
    "name": "Orange Vivid",
    "colors": [
      "#e8ebed",
      "#e37e31",
      "#1a1a1a"
    ]
  }
] as const;
export type Preset = typeof PRESETS[number]["id"];
export type Mode = "system" | "light" | "dark";
/** A workspace preset, or the default Omniget desktop style. */
export type DesignPreset = Preset | typeof OMNIGET_PRESET;

export function getWorkspaceDesign(): { preset: DesignPreset; mode: Mode; systemDark: boolean; resolvedMode: "light" | "dark" } {
  const p = getAppearancePresentation();
  const preset = (PRESETS.some((item) => item.id === p.preset) ? p.preset : OMNIGET_PRESET) as DesignPreset;
  return { preset, mode: p.mode, systemDark: p.systemDark, resolvedMode: p.resolvedMode };
}

/** Kept for callers of the old API; the store applies and follows the OS itself. */
export function initWorkspaceDesign(): () => void {
  return () => {};
}

/** Saves mode and preset through the settings store; true only after confirmation. */
export async function saveWorkspaceDesign(nextPreset: DesignPreset, nextMode: Mode): Promise<boolean> {
  try {
    await updateSettings({ appearance: { preset: nextPreset, mode: nextMode } });
    return true;
  } catch {
    return false;
  }
}


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


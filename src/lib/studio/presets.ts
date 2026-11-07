import type { CaptionStyle, PresetId } from "./types";

export const PRESET_IDS: PresetId[] = ["clean", "bold_pop", "boxed", "minimal"];

export const BUNDLED_FONT = "Liberation Sans";

export const FONTS = [BUNDLED_FONT, "Arial", "Helvetica Neue", "Verdana", "Impact", "Georgia", "Courier New"];

const BASE: CaptionStyle = {
  preset: "clean",
  font: BUNDLED_FONT,
  size: 64,
  bold: true,
  color: "#FFFFFF",
  highlight: "#FFD60A",
  karaoke: false,
  outline: 4,
  outlineColor: "#000000",
  box: false,
  boxColor: "#000000",
  shadow: 2,
  position: "bottom",
  align: "center",
  maxChars: 32,
  maxLines: 2,
  uppercase: false,
};

export const PRESETS: Record<PresetId, CaptionStyle> = {
  clean: { ...BASE },
  bold_pop: {
    ...BASE,
    preset: "bold_pop",
    size: 84,
    karaoke: true,
    outline: 6,
    shadow: 0,
    position: "middle",
    maxChars: 18,
    maxLines: 2,
    uppercase: true,
  },
  boxed: {
    ...BASE,
    preset: "boxed",
    size: 56,
    bold: true,
    outline: 12,
    box: true,
    boxColor: "#000000",
    shadow: 0,
    maxChars: 30,
  },
  minimal: {
    ...BASE,
    preset: "minimal",
    size: 44,
    bold: false,
    outline: 0,
    shadow: 3,
    align: "left",
    maxChars: 40,
    maxLines: 1,
  },
};

export function applyPreset(id: PresetId): CaptionStyle {
  return { ...PRESETS[id] };
}

export const ASS_SIZE_TO_CSS = 1 / 1.117;

export const CAPTION_SHADOW = "rgb(0 0 0 / 0.5)";

export type Alignment = 1 | 2 | 4 | 5 | 7 | 8;

export type Geometry = {
  fontSize: number;
  outline: number;
  shadow: number;
  marginV: number;
  marginH: number;
  alignment: Alignment;
};

export function geometry(style: CaptionStyle, width: number, height: number): Geometry {
  const k = Math.min(width, height) / 1080;
  const lowerThird = style.preset === "minimal";
  const marginV = Math.round(height * (lowerThird && style.position === "bottom" ? 0.12 : 0.08));
  const marginH = Math.round(width * 0.06);
  const left = style.align === "left";
  const alignment: Alignment =
    style.position === "top" ? (left ? 7 : 8) : style.position === "middle" ? (left ? 4 : 5) : left ? 1 : 2;
  return {
    fontSize: Math.max(8, Math.round(style.size * k)),
    outline: Math.max(0, Math.round(style.outline * k * 10) / 10),
    shadow: Math.max(0, Math.round(style.shadow * k * 10) / 10),
    marginV,
    marginH,
    alignment,
  };
}

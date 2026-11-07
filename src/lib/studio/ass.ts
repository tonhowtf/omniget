import type { Cue } from "./layout";
import { geometry } from "./presets";
import type { CaptionStyle } from "./types";

export function assColor(hex: string, alpha = 0): string {
  const m = hex.trim().match(/^#?([0-9a-f]{6})$/i);
  const v = m ? m[1].toUpperCase() : "FFFFFF";
  const a = Math.max(0, Math.min(255, Math.round(alpha))).toString(16).toUpperCase().padStart(2, "0");
  return `&H${a}${v.slice(4, 6)}${v.slice(2, 4)}${v.slice(0, 2)}`;
}

export function assTime(ms: number): string {
  const cs = Math.max(0, Math.round(ms / 10));
  const h = Math.floor(cs / 360000);
  const m = Math.floor((cs % 360000) / 6000);
  const s = Math.floor((cs % 6000) / 100);
  const c = cs % 100;
  return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}.${String(c).padStart(2, "0")}`;
}

export function escapeAssText(text: string): string {
  return text.replace(/\\/g, "⧵").replace(/\{/g, "(").replace(/\}/g, ")").replace(/[\r\n]+/g, " ");
}

function fontName(name: string): string {
  return name.replace(/,/g, " ").trim() || "Liberation Sans";
}

export function karaokeText(cue: Cue): string {
  let cursor = Math.round(cue.start_ms / 10);
  const words = cue.lines.flat();
  const lead = Math.round(words[0].start_ms / 10) - cursor;
  let out = lead > 0 ? `{\\k${lead}}` : "";
  if (lead > 0) cursor += lead;
  const total = words.length;
  let index = 0;
  const lines = cue.lines.map((line) =>
    line
      .map((w) => {
        index += 1;
        const next = index < total ? words[index].start_ms : cue.end_ms;
        const end = Math.max(cursor, Math.round(next / 10));
        const k = end - cursor;
        cursor = end;
        const space = index < total && line[line.length - 1] !== w ? " " : "";
        return `{\\k${k}}${escapeAssText(w.text)}${space}`;
      })
      .join(""),
  );
  out += lines.join("\\N");
  return out;
}

export function plainCueText(cue: Cue): string {
  return cue.lines.map((line) => line.map((w) => escapeAssText(w.text)).join(" ")).join("\\N");
}

export function buildAss(cues: Cue[], style: CaptionStyle, width: number, height: number): string {
  const g = geometry(style, width, height);
  const primary = style.karaoke ? assColor(style.highlight) : assColor(style.color);
  const secondary = assColor(style.color);
  const outlineColour = style.box ? assColor(style.boxColor) : assColor(style.outlineColor);
  const back = assColor("#000000", 0x80);
  const borderStyle = style.box ? 3 : 1;
  const styleLine = [
    "Default",
    fontName(style.font),
    g.fontSize,
    primary,
    secondary,
    outlineColour,
    back,
    style.bold ? -1 : 0,
    0,
    0,
    0,
    100,
    100,
    0,
    0,
    borderStyle,
    g.outline,
    style.box ? 0 : g.shadow,
    g.alignment,
    g.marginH,
    g.marginH,
    g.marginV,
    1,
  ].join(",");
  const events = cues
    .filter((c) => c.lines.length && c.end_ms > c.start_ms)
    .map((c) => `Dialogue: 0,${assTime(c.start_ms)},${assTime(c.end_ms)},Default,,0,0,0,,${style.karaoke ? karaokeText(c) : plainCueText(c)}`);
  return [
    "[Script Info]",
    "ScriptType: v4.00+",
    `PlayResX: ${width}`,
    `PlayResY: ${height}`,
    "WrapStyle: 2",
    "ScaledBorderAndShadow: yes",
    "YCbCr Matrix: None",
    "",
    "[V4+ Styles]",
    "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding",
    `Style: ${styleLine}`,
    "",
    "[Events]",
    "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text",
    ...events,
    "",
  ].join("\n");
}

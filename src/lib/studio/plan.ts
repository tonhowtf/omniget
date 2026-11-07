import { buildAss } from "./ass";
import { introLines, outputSize } from "./frame";
import { layoutCues } from "./layout";
import type { CaptionStyle, IntroSettings, Probe, Quality, Reframe, RenderRequest, Segment } from "./types";

export type PlanInput = {
  probe: Probe;
  segments: Segment[];
  burnCaptions: boolean;
  style: CaptionStyle;
  reframe: Reframe;
  intro: IntroSettings;
  fadeOut: boolean;
  quality: Quality;
  outDir: string | null;
};

export function captionsUsable(segments: Segment[]): boolean {
  return segments.some((s) => s.text.trim().length > 0);
}

export function introUsable(intro: IntroSettings): boolean {
  return intro.enabled && (intro.title.trim().length > 0 || intro.subtitle.trim().length > 0);
}

export function buildRequest(p: PlanInput): RenderRequest {
  const [w, h] = outputSize(p.probe.width, p.probe.height, p.reframe.aspect, p.reframe.mode);
  const cues = layoutCues(p.segments, { maxChars: p.style.maxChars, maxLines: p.style.maxLines, uppercase: p.style.uppercase });
  const ass = p.burnCaptions && cues.length > 0 ? buildAss(cues, p.style, w, h) : null;
  const lines = introLines(w, h, p.intro.title, p.intro.subtitle);
  return {
    input: p.probe.path,
    out_dir: p.outDir,
    ass,
    reframe: { ...p.reframe },
    intro: introUsable(p.intro)
      ? {
          seconds: Math.min(15, Math.max(1, p.intro.seconds)),
          title_lines: lines.title,
          subtitle_lines: lines.subtitle,
          background: p.intro.background,
          color: p.intro.color,
          fade: p.intro.fade,
        }
      : null,
    fade_out: p.fadeOut,
    quality: p.quality,
  };
}

export function nothingToChange(p: PlanInput): boolean {
  const req = buildRequest(p);
  return !req.ass && !req.intro && req.reframe.aspect === "keep" && !req.fade_out;
}

export function stemOf(name: string): string {
  return name.replace(/\.[^./\\]+$/, "") || "captions";
}

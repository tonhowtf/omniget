export type Mood =
  | "neutral"
  | "happy"
  | "curious"
  | "proud"
  | "worried"
  | "sleepy"
  | "focused";

export const MOODS: readonly Mood[] = [
  "neutral",
  "happy",
  "curious",
  "proud",
  "worried",
  "sleepy",
  "focused",
];

export type OmniPose = "idle" | "talk" | "wave" | "sleep" | "work";

interface PoseSlice {
  start: number;
  count: number;
  fps: number;
  loop: boolean;
}

interface SpriteStrip {
  image: string;
  frameW: number;
  frameH: number;
  frames: number;
  poses: Record<OmniPose, PoseSlice>;
}

export const OMNI_STRIP: SpriteStrip = {
  image: "/omni/poses/omni-strip.png",
  frameW: 48,
  frameH: 64,
  frames: 26,
  poses: {
    idle: { start: 0, count: 4, fps: 6, loop: true },
    talk: { start: 4, count: 4, fps: 8, loop: true },
    wave: { start: 8, count: 6, fps: 10, loop: true },
    sleep: { start: 14, count: 4, fps: 4, loop: true },
    work: { start: 18, count: 8, fps: 10, loop: true },
  },
};

export function moodPose(mood: Mood | string | null | undefined, speaking = false): OmniPose {
  if (speaking) return "talk";
  switch (mood) {
    case "happy":
    case "proud":
      return "wave";
    case "sleepy":
      return "sleep";
    case "focused":
      return "work";
    case "curious":
    case "worried":
    case "neutral":
      return "idle";
    default:
      return "idle";
  }
}

function poseSlice(strip: SpriteStrip, pose: OmniPose | string): PoseSlice {
  return strip.poses[pose as OmniPose] ?? strip.poses.idle;
}

function leftOf(px: number): number {
  return px === 0 ? 0 : -px;
}

interface StripAnimation {
  fromPx: number;
  toPx: number;
  steps: number;
  durationMs: number;
  widthPx: number;
  frameW: number;
  frameH: number;
  image: string;
}

export function stripAnimationCss(
  strip: SpriteStrip,
  pose: OmniPose | string,
): StripAnimation {
  const slice = poseSlice(strip, pose);
  const start = slice.start * strip.frameW;
  const fps = slice.fps > 0 ? slice.fps : 1;
  return {
    fromPx: leftOf(start),
    toPx: leftOf(start + slice.count * strip.frameW),
    steps: slice.count,
    durationMs: Math.round((slice.count / fps) * 1000),
    widthPx: strip.frames * strip.frameW,
    frameW: strip.frameW,
    frameH: strip.frameH,
    image: strip.image,
  };
}


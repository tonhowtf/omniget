// Expressions: the body carries the 52 ARKit facial actions as morph targets
// ("perfect sync"); VRM presets and moods are weighted mixes of them.

export const VRM_PRESETS: Record<string, Record<string, number>> = {
  happy: { mouthSmileLeft: 1, mouthSmileRight: 1, cheekSquintLeft: 0.5, cheekSquintRight: 0.5, eyeSquintLeft: 0.3, eyeSquintRight: 0.3 },
  angry: { browDownLeft: 1, browDownRight: 1, noseSneerLeft: 0.6, noseSneerRight: 0.6, mouthFrownLeft: 0.4, mouthFrownRight: 0.4 },
  sad: { mouthFrownLeft: 1, mouthFrownRight: 1, browInnerUp: 1 },
  relaxed: { mouthSmileLeft: 0.4, mouthSmileRight: 0.4, eyeSquintLeft: 0.25, eyeSquintRight: 0.25 },
  surprised: { browInnerUp: 1, browOuterUpLeft: 1, browOuterUpRight: 1, eyeWideLeft: 1, eyeWideRight: 1, jawOpen: 0.4 },
  aa: { jawOpen: 0.7 },
  ih: { mouthStretchLeft: 0.6, mouthStretchRight: 0.6, jawOpen: 0.2 },
  ou: { mouthPucker: 1, mouthFunnel: 0.5 },
  ee: { mouthStretchLeft: 1, mouthStretchRight: 1, jawOpen: 0.1 },
  oh: { mouthFunnel: 1, jawOpen: 0.4 },
  blink: { eyeBlinkLeft: 1, eyeBlinkRight: 1 },
  blinkLeft: { eyeBlinkLeft: 1 },
  blinkRight: { eyeBlinkRight: 1 },
  lookUp: { eyeLookUpLeft: 1, eyeLookUpRight: 1 },
  lookDown: { eyeLookDownLeft: 1, eyeLookDownRight: 1 },
  lookLeft: { eyeLookOutLeft: 1, eyeLookInRight: 1 },
  lookRight: { eyeLookInLeft: 1, eyeLookOutRight: 1 },
};

/** Mood (catalog `moods`) to a resting face. */
export const MOOD_FACE: Record<string, Record<string, number>> = {
  neutral: {},
  happy: { happy: 0.45 },
  sad: { sad: 0.5 },
  relaxed: { relaxed: 0.6 },
  angry: { angry: 0.4 },
  surprised: { surprised: 0.3 },
  tired: { blink: 0.35, sad: 0.15 },
  curious: { surprised: 0.15, happy: 0.15 },
};

/** Expands VRM preset names and raw ARKit names into ARKit weights. */
export function toArkit(weights: Record<string, number>): Record<string, number> {
  const out: Record<string, number> = {};
  for (const [name, w] of Object.entries(weights)) {
    const preset = VRM_PRESETS[name];
    if (preset) {
      for (const [k, v] of Object.entries(preset)) out[k] = Math.max(out[k] ?? 0, v * w);
    } else {
      out[name] = Math.max(out[name] ?? 0, w);
    }
  }
  return out;
}

// 10-band graphic EQ (ISO octave centres) and its presets.

export const EQ_FREQS = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000] as const;

export type EqPreset = { id: string; gains: number[] };

export const EQ_PRESETS: EqPreset[] = [
  { id: "flat", gains: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0] },
  { id: "bass", gains: [6, 5, 4, 2, 0, 0, 0, 0, 0, 0] },
  { id: "treble", gains: [0, 0, 0, 0, 0, 1, 2, 4, 5, 6] },
  { id: "vocal", gains: [-2, -2, -1, 1, 3, 4, 3, 1, 0, -1] },
  { id: "rock", gains: [4, 3, 2, 0, -1, -1, 1, 3, 4, 4] },
  { id: "pop", gains: [-1, 1, 3, 4, 3, 0, -1, -1, 0, 1] },
  { id: "electronic", gains: [5, 4, 1, 0, -2, 1, 0, 1, 4, 5] },
  { id: "jazz", gains: [3, 2, 1, 2, -1, -1, 0, 1, 2, 3] },
  { id: "classical", gains: [4, 3, 2, 1, -1, -1, 0, 2, 3, 4] },
  { id: "loudness", gains: [5, 4, 1, 0, -1, 0, 0, 1, 3, 4] },
  { id: "night", gains: [-4, -3, -1, 0, 1, 2, 2, 0, -2, -3] },
];

/** Headroom the preamp should pull so boosted bands don't clip. */
export function autoPreamp(gains: number[]): number {
  const max = Math.max(0, ...gains);
  return -Math.round(max * 0.8 * 10) / 10;
}

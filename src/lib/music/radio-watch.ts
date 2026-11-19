import type { Os } from "$lib/platform";

export type RadioVerdict = "ok" | "stalled" | "silent";

export type RadioSample = {
  now: number;
  time: number;
  paused: boolean;
  level: number;
  routed: boolean;
  contextRunning: boolean;
};

export function createRadioWatch({ stallMs = 20_000, silenceMs = 8_000 } = {}) {
  let lastTime = -1;
  let lastMove: number | null = null;
  let lastSound: number | null = null;
  return {
    step(s: RadioSample): RadioVerdict {
      if (s.paused) {
        lastMove = null;
        lastSound = null;
        return "ok";
      }
      if (lastMove === null || lastSound === null) {
        lastTime = s.time;
        lastMove = s.now;
        lastSound = s.now;
        return "ok";
      }
      const moved = s.time > lastTime + 0.01;
      if (moved) {
        lastTime = s.time;
        lastMove = s.now;
      }
      if (s.now - lastMove >= stallMs) return "stalled";
      const audible = !s.routed || (s.contextRunning && s.level > 0);
      if (audible || !moved) lastSound = s.now;
      return s.now - lastSound >= silenceMs ? "silent" : "ok";
    },
  };
}

export function radioUsesGraph(os: Os): boolean {
  return os !== "windows";
}

export const RADIO_RETRY_LIMIT = 2;
export const RADIO_RETRY_RESET_MS = 60_000;

export function nextRadioRetry(retries: number, msSinceConnect: number): number | null {
  const used = msSinceConnect >= RADIO_RETRY_RESET_MS ? 0 : retries;
  return used >= RADIO_RETRY_LIMIT ? null : used + 1;
}

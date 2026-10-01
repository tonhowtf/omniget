/**
 * Pace: is a window being spent faster than it refills? A port of
 * `src-tauri/src/limits_strip/pace.rs` (after CodexBar's `UsagePace`): the
 * share of the window used against the share of its time gone, extrapolated
 * linearly. Pure functions; the caller passes the clock.
 */

/** Below this share of the window gone, a forecast is noise. */
const MIN_SHOWN = 0.03;
/** Within this much of the tick, the window reads as "on pace". */
const ON_PACE = 0.03;

type Pace = {
  /** Share of the window's time gone, 0..1: where the "on pace" tick sits. */
  expected: number;
  /** `used - expected`: positive = ahead of pace (spending too fast). */
  delta: number;
  /** When the window runs out at this pace, if before the reset. */
  runsOutAt: number | null;
};

/** What a window needs for a pace; `LimitWindow` fits. */
type Paced = { used: number | null; resets_at: number | null; span_ms?: number | null };

/**
 * Share of the window's time already gone, 0..1, when the reset and the
 * length are known and consistent.
 */
export function elapsed(resetsAt: number, spanMs: number, now: number): number | null {
  if (!(spanMs > 0)) return null;
  const untilReset = resetsAt - now;
  if (untilReset <= 0 || untilReset > spanMs) return null;
  return (spanMs - untilReset) / spanMs;
}

/**
 * Epoch ms at which the window runs out at the current pace, or null when it
 * lasts until the reset (or nothing can be said yet).
 */
export function runsOutAt(used: number, resetsAt: number, spanMs: number, now: number): number | null {
  const gone = elapsed(resetsAt, spanMs, now);
  if (gone == null) return null;
  const actual = Math.max(0, Math.min(1, used));
  if (actual >= 1) return now;
  if (gone <= 0 || actual <= 0) return null;
  const elapsedMs = gone * spanMs;
  const leftMs = (1 - actual) / (actual / elapsedMs);
  return leftMs < resetsAt - now ? now + Math.trunc(leftMs) : null;
}

/** The pace of one window, or null with no reset, no length or too little history. */
export function paceOf(w: Paced, now: number): Pace | null {
  if (w.used == null || w.resets_at == null || w.span_ms == null) return null;
  const expected = elapsed(w.resets_at, w.span_ms, now);
  if (expected == null || expected < MIN_SHOWN) return null;
  return {
    expected,
    delta: w.used - expected,
    runsOutAt: runsOutAt(w.used, w.resets_at, w.span_ms, now),
  };
}

/** Which way a delta leans, with a dead band around the tick. */
export function paceSide(delta: number): "ahead" | "under" | "on" {
  if (delta > ON_PACE) return "ahead";
  return delta < -ON_PACE ? "under" : "on";
}

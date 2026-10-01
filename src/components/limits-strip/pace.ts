const MIN_SHOWN = 0.03;
const ON_PACE = 0.03;

type Pace = {
  expected: number;
  delta: number;
  runsOutAt: number | null;
};

type Paced = { used: number | null; resets_at: number | null; span_ms?: number | null };

export function elapsed(resetsAt: number, spanMs: number, now: number): number | null {
  if (!(spanMs > 0)) return null;
  const untilReset = resetsAt - now;
  if (untilReset <= 0 || untilReset > spanMs) return null;
  return (spanMs - untilReset) / spanMs;
}

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

export function paceSide(delta: number): "ahead" | "under" | "on" {
  if (delta > ON_PACE) return "ahead";
  return delta < -ON_PACE ? "under" : "on";
}

import { describe, expect, it } from "vitest";
import { createRadioWatch, nextRadioRetry, RADIO_RETRY_LIMIT, type RadioSample } from "./radio-watch";

function sample(over: Partial<RadioSample>): RadioSample {
  return { now: 0, time: 0, paused: false, level: 0.2, routed: true, contextRunning: true, ...over };
}

describe("radio watch", () => {
  it("keeps quiet while the stream moves and makes sound", () => {
    const w = createRadioWatch();
    for (let i = 0; i < 30; i++) {
      expect(w.step(sample({ now: i * 1000, time: i }))).toBe("ok");
    }
  });

  it("reports a stream that stopped moving while it still says playing", () => {
    const w = createRadioWatch({ stallMs: 5000 });
    expect(w.step(sample({ now: 0, time: 3 }))).toBe("ok");
    expect(w.step(sample({ now: 4000, time: 3 }))).toBe("ok");
    expect(w.step(sample({ now: 5000, time: 3 }))).toBe("stalled");
  });

  it("reports a moving stream that reaches the output with no signal", () => {
    const w = createRadioWatch({ silenceMs: 3000 });
    w.step(sample({ now: 0, time: 0, level: 0 }));
    expect(w.step(sample({ now: 1000, time: 1, level: 0 }))).toBe("ok");
    expect(w.step(sample({ now: 3000, time: 3, level: 0 }))).toBe("silent");
  });

  it("reports a suspended audio context even when the analyser reads a level", () => {
    const w = createRadioWatch({ silenceMs: 2000 });
    w.step(sample({ now: 0, time: 0, contextRunning: false }));
    expect(w.step(sample({ now: 2000, time: 2, contextRunning: false }))).toBe("silent");
  });

  it("does not call silence on an element playing outside the audio graph", () => {
    const w = createRadioWatch({ silenceMs: 1000 });
    w.step(sample({ now: 0, time: 0, level: 0, routed: false }));
    expect(w.step(sample({ now: 5000, time: 5, level: 0, routed: false }))).toBe("ok");
  });

  it("forgets the clock while paused so resuming is not a stall", () => {
    const w = createRadioWatch({ stallMs: 1000 });
    w.step(sample({ now: 0, time: 1 }));
    expect(w.step(sample({ now: 60_000, time: 1, paused: true }))).toBe("ok");
    expect(w.step(sample({ now: 60_500, time: 1 }))).toBe("ok");
  });
});

describe("radio retries", () => {
  it("gives up after the limit on a station that keeps dropping", () => {
    expect(nextRadioRetry(0, 1000)).toBe(1);
    expect(nextRadioRetry(RADIO_RETRY_LIMIT, 1000)).toBeNull();
  });

  it("starts over after a long healthy run", () => {
    expect(nextRadioRetry(RADIO_RETRY_LIMIT, 120_000)).toBe(1);
  });
});

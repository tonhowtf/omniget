import { describe, expect, it } from "vitest";
import { serialRunner } from "./serial";

describe("serialRunner", () => {
  it("never runs two tasks at once and always runs once more after a late call", async () => {
    let active = 0;
    let peak = 0;
    let state = 0;
    const written: number[] = [];
    const run = serialRunner(async () => {
      active++;
      peak = Math.max(peak, active);
      const seen = state;
      await new Promise((r) => setTimeout(r, 5));
      written.push(seen);
      active--;
    });
    state = 1;
    const first = run();
    state = 2;
    const second = run();
    state = 3;
    const third = run();
    await Promise.all([first, second, third]);
    expect(peak).toBe(1);
    expect(written).toEqual([1, 3]);
    state = 4;
    await run();
    expect(written).toEqual([1, 3, 4]);
  });
  it("keeps working after a task throws", async () => {
    let calls = 0;
    const run = serialRunner(async () => {
      calls++;
      if (calls === 1) throw new Error("boom");
    });
    await expect(run()).rejects.toThrow("boom");
    await run();
    expect(calls).toBe(2);
  });
});

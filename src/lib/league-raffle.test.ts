import { describe, it, expect } from "vitest";
import { CHAMPION_CLASSES, LANES, drawLane, secondaryLane } from "./league-raffle";

describe("drawLane", () => {
  it("never repeats the previous lane when there is a choice", () => {
    for (let i = 0; i < 200; i++) {
      expect(drawLane("TOP")).not.toBe("TOP");
    }
  });

  it("reaches every lane", () => {
    const seen = new Set<string>();
    for (let i = 0; i < 500; i++) seen.add(drawLane(null));
    expect([...seen].sort()).toEqual([...LANES].sort());
  });

  it("is deterministic given the random source", () => {
    expect(drawLane(null, () => 0)).toBe("TOP");
    expect(drawLane(null, () => 0.999)).toBe("UTILITY");
    // The previous lane is removed from the pool before indexing.
    expect(drawLane("TOP", () => 0)).toBe("JUNGLE");
  });
});

describe("secondaryLane", () => {
  it("fills half the time and otherwise draws a different lane", () => {
    expect(secondaryLane("MIDDLE", () => 0.2)).toBe("FILL");
    const second = secondaryLane("MIDDLE", () => 0.9);
    expect(second).not.toBe("FILL");
    expect(second).not.toBe("MIDDLE");
  });
});

describe("champion classes", () => {
  it("match the tags the client uses", () => {
    expect(CHAMPION_CLASSES).toContain("marksman");
    expect(CHAMPION_CLASSES).toHaveLength(6);
  });
});

import { describe, expect, it } from "vitest";
import { groupByDay, isoToLocalInput, localInputToIso, rejection, sortSlots, textLimit, weightedLength } from "./schedule";

describe("weightedLength", () => {
  it("counts like the Rust rule", () => {
    expect(weightedLength("hello")).toBe(5);
    expect(weightedLength("olá, São Paulo")).toBe(14);
    expect(weightedLength("日本語")).toBe(6);
    expect(weightedLength("a 🔥")).toBe(4);
    expect(weightedLength("👨‍👩‍👧")).toBe(2);
    expect(weightedLength("👍🏽")).toBe(2);
    expect(weightedLength("🇧🇷")).toBe(2);
    expect(weightedLength("1️⃣")).toBe(2);
    expect(weightedLength("read https://example.com/a/very/long/path/that/goes/on/and/on?x=1")).toBe(28);
    expect(weightedLength("see example.com.")).toBe(28);
    expect(weightedLength("é")).toBe(1);
    expect(weightedLength(`${"a".repeat(256)} https://example.com/${"b".repeat(300)}`)).toBe(280);
  });
});

describe("textLimit", () => {
  it("follows the setting and the account", () => {
    expect(textLimit("auto", false)).toBe(280);
    expect(textLimit("auto", true)).toBe(4000);
    expect(textLimit("on", false)).toBe(4000);
    expect(textLimit("off", true)).toBe(280);
  });
});

describe("times", () => {
  it("turns a datetime-local value into ISO with this computer's offset and back", () => {
    const iso = localInputToIso("2026-11-12T09:30");
    expect(iso).toMatch(/^2026-11-12T09:30:00[+-]\d{2}:\d{2}$/);
    expect(new Date(iso as string).getTime()).toBe(new Date("2026-11-12T09:30").getTime());
    expect(isoToLocalInput(iso)).toBe("2026-11-12T09:30");
    expect(localInputToIso("")).toBeNull();
    expect(localInputToIso("nope")).toBeNull();
    expect(isoToLocalInput(null)).toBe("");
  });

  it("groups consecutive items by local day and skips items without a time", () => {
    const a = localInputToIso("2026-11-12T09:30");
    const b = localInputToIso("2026-11-12T23:59");
    const c = localInputToIso("2026-11-13T00:01");
    const groups = groupByDay([{ at: a }, { at: b }, { at: null }, { at: c }]);
    expect(groups.map((g) => [g.day, g.items.length])).toEqual([
      ["2026-11-12", 2],
      ["2026-11-13", 1],
    ]);
  });
});

describe("rejection", () => {
  it("reads the code and the JSON detail", () => {
    expect(rejection('PAST_TIME: {"message":"the time is in the past","retryable":false,"next_step":"pass a future time"}')).toEqual({
      code: "PAST_TIME",
      message: "the time is in the past",
      next_step: "pass a future time",
    });
    expect(rejection("NOT_LOGGED_IN: sign in to X in OmniGet first")).toEqual({ code: "NOT_LOGGED_IN", message: "sign in to X in OmniGet first", next_step: "" });
    expect(rejection(new Error("boom"))).toEqual({ code: "UNKNOWN", message: "boom", next_step: "" });
  });
});

describe("sortSlots", () => {
  it("orders by weekday then time", () => {
    expect(sortSlots([{ weekday: "wed", time: "18:00" }, { weekday: "mon", time: "09:30" }, { weekday: "wed", time: "08:00" }]).map((s) => `${s.weekday} ${s.time}`)).toEqual(["mon 09:30", "wed 08:00", "wed 18:00"]);
  });
});

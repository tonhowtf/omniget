import { describe, expect, it } from "vitest";
import type { TubeSettings } from "$lib/tube/types";
import { activePreset, presetDiff, presetKeys, presetPatch } from "./presets";
import { cleanRule, minutesToSeconds, newRule, parseWords, ruleProblems, secondsToMinutes, usageRatio } from "./rules";

// Only the fields presets look at; the rest is irrelevant here.
const base = {
  start_page: "home",
  hide_shorts: false,
  hide_members: true,
  hide_mixes: false,
  hide_playables: true,
  hide_end_screen: true,
  hide_home_feed: false,
  hide_related: false,
  hide_watched: false,
  hide_comments: false,
  watched_threshold: 85,
} as unknown as TubeSettings;

describe("distraction presets", () => {
  it("defaults are custom until a preset is applied", () => {
    expect(activePreset(base)).toBeNull();
    expect(activePreset({ ...base, ...presetPatch("clean") })).toBe("clean");
  });

  it("detects the most specific preset", () => {
    expect(activePreset({ ...base, ...presetPatch("focus") })).toBe("focus");
    expect(activePreset({ ...base, ...presetPatch("seeker") })).toBe("seeker");
  });

  it("focus builds on clean and seeker on focus", () => {
    const focus = presetPatch("focus");
    const seeker = presetPatch("seeker");
    for (const k of presetKeys("clean")) expect(k in focus).toBe(true);
    for (const k of presetKeys("focus")) expect(k in seeker).toBe(true);
    expect(focus.hide_home_feed).toBe(true);
    expect(focus.hide_shorts).toBe(true);
    expect(seeker.hide_comments).toBe(true);
    expect(seeker.start_page).toBe("subscriptions");
    expect(seeker.hide_related).toBe(true);
  });

  it("changing one option drops back to custom", () => {
    const s = { ...base, ...presetPatch("focus"), hide_related: false };
    expect(activePreset(s)).toBeNull();
  });

  it("counts what a preset would change", () => {
    expect(presetDiff({ ...base, ...presetPatch("clean") }, "clean")).toBe(0);
    expect(presetDiff(base, "clean")).toBe(1); // hide_mixes
  });

  it("returns copies", () => {
    const p = presetPatch("clean");
    p.hide_mixes = false;
    expect(presetPatch("clean").hide_mixes).toBe(true);
  });
});

describe("auto-download rules", () => {
  it("parses word lists", () => {
    expect(parseWords(" live, Podcast\npodcast ;; trailer ,")).toEqual(["live", "Podcast", "trailer"]);
    expect(parseWords("")).toEqual([]);
  });

  it("converts minutes", () => {
    expect(minutesToSeconds("10")).toBe(600);
    expect(minutesToSeconds("1,5")).toBe(90);
    expect(minutesToSeconds("")).toBeNull();
    expect(minutesToSeconds("-3")).toBeNull();
    expect(secondsToMinutes(600)).toBe("10");
    expect(secondsToMinutes(90)).toBe("1.5");
    expect(secondsToMinutes(null)).toBe("");
  });

  it("validates", () => {
    const r = newRule();
    expect(ruleProblems(r)).toEqual(["name"]);
    expect(ruleProblems({ ...r, name: "x", min_duration_s: 600, max_duration_s: 60 })).toEqual(["duration"]);
    expect(ruleProblems({ ...r, name: "x", max_gb: -1 })).toEqual(["numbers"]);
    expect(ruleProblems({ ...r, name: "x" })).toEqual([]);
  });

  it("cleans before saving", () => {
    const r = cleanRule({ ...newRule(" News "), include_words: ["a", " A ", ""], folder: "  ", min_duration_s: 0, max_age_days: 0 });
    expect(r.name).toBe("News");
    expect(r.include_words).toEqual(["a"]);
    expect(r.folder).toBeNull();
    expect(r.min_duration_s).toBeNull();
    expect(r.max_age_days).toBeNull();
  });

  it("new rules get unique ids", () => {
    expect(newRule().id).not.toBe(newRule().id);
  });

  it("usage ratio", () => {
    expect(usageRatio(0, 0)).toBe(0);
    expect(usageRatio(512 * 1024 ** 2, 1)).toBeCloseTo(0.5);
    expect(usageRatio(5 * 1024 ** 3, 1)).toBe(1);
  });
});
